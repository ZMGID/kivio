//! 笔记中心：每篇笔记是 `<app_data>/notes` 下的一个 Markdown 文件。
//! 应用创建的笔记按标题命名，稳定 ID 与标题、时间戳、文件夹、来源一同存于 frontmatter。
//! 旧笔记与外部 Markdown 可用文件名作为 ID。frontmatter 解析复用
//! skills 的 `split_frontmatter`，与 agents / skills 的文件格式约定保持一致。
//!
//! 组织方式：
//! - `origin`：`user`（手动新建）| `chat`（对话存来）。缺省 = user，仅记录来源。
//! - `folder`：单层文件夹名，空 = 库根。归档位置与 origin 来源独立。
//! - 空文件夹也要留存，故文件夹清单单独存 `<app_data>/notes/folders.json`（有序数组）。
//! 文件始终扁平存放，移动文件夹只改 frontmatter。

use std::{
    collections::{HashMap, HashSet},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    str::FromStr,
    sync::{Mutex, MutexGuard},
    time::SystemTime,
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tauri_plugin_shell::ShellExt;
use uuid::Uuid;

use crate::{
    chat::storage::atomic_write, skills::parse::split_frontmatter, utils::rename_no_replace,
};

// Preserve serialized note/folder writes after moving IPC work off the UI thread.
static NOTE_WRITES: Mutex<()> = Mutex::new(());
pub(crate) const DIRECTORY: &str = "notes";

pub(crate) fn lock_storage() -> MutexGuard<'static, ()> {
    NOTE_WRITES.lock().unwrap_or_else(|e| e.into_inner())
}

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

struct NoteFile {
    note: Note,
    raw: String,
}

#[derive(Debug, Deserialize)]
pub struct NoteFieldEdit {
    before: String,
    after: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct NoteEdit {
    title: Option<NoteFieldEdit>,
    content: Option<NoteFieldEdit>,
    folder: Option<NoteFieldEdit>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum NoteWriteError {
    Conflict { field: &'static str },
    Storage { message: String },
}

impl From<String> for NoteWriteError {
    fn from(message: String) -> Self {
        Self::Storage { message }
    }
}

/// 用户笔记目录：`<app_data>/notes`。按需创建。
fn notes_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|err| format!("app_data_dir unavailable: {err}"))?
        .join(DIRECTORY);
    std::fs::create_dir_all(&dir).map_err(|err| format!("create notes dir failed: {err}"))?;
    Ok(dir)
}

fn folders_file(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(notes_dir(app)?.join("folders.json"))
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// 兼容用文件名作为 ID 的笔记。
fn validate_note_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.contains(['/', '\\', '\0'])
        || id.contains("..")
        || (cfg!(windows) && id.contains(':'))
    {
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

fn note_file_stem(title: &str) -> String {
    let safe = title.replace(
        |ch: char| {
            ch.is_control() || matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        },
        "-",
    );
    let safe = safe.trim_matches([' ', '.', '-']);
    let stem = safe[..safe.floor_char_boundary(120)].trim_end_matches([' ', '.', '-']);
    if stem.is_empty() {
        return "无标题笔记".into();
    }
    let device = stem
        .split('.')
        .next()
        .unwrap_or(stem)
        .trim_end()
        .to_ascii_uppercase();
    let numbered_device = device
        .strip_prefix("COM")
        .or_else(|| device.strip_prefix("LPT"))
        .is_some_and(|number| {
            matches!(
                number,
                "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        });
    if numbered_device
        || Uuid::try_parse(stem).is_ok()
        || matches!(
            device.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        )
    {
        return format!("笔记 - {stem}");
    }
    stem.to_string()
}

fn publish_note(source: &Path, dir: &Path, stem: &str) -> Result<PathBuf, String> {
    for number in 1..=u32::MAX {
        let filename = if number == 1 {
            format!("{stem}.md")
        } else {
            format!("{stem} ({number}).md")
        };
        let target = dir.join(filename);
        if source == target {
            return Ok(target);
        }
        match rename_no_replace(source, &target) {
            Ok(()) => return Ok(target),
            Err(err)
                if err.kind() == io::ErrorKind::AlreadyExists
                    || target.symlink_metadata().is_ok() => {}
            Err(err) => return Err(format!("name note file failed: {err}")),
        }
    }
    Err("no available note file name".to_string())
}

fn serialize_note(note: &Note) -> String {
    let mut out = format!(
        "---\ntitle: {}\ncreated_at: {}\nupdated_at: {}\n",
        yaml_string(&note.title),
        note.created_at,
        note.updated_at,
    );
    if Uuid::try_parse(&note.id).is_ok() {
        out.push_str(&format!("kivio_note_id: {}\n", note.id));
    }
    let folder = sanitize_line(&note.folder);
    if !folder.is_empty() {
        out.push_str(&format!("folder: {}\n", yaml_string(&folder)));
    }
    if note.origin == ORIGIN_CHAT {
        out.push_str("origin: chat\n");
    }
    out.push_str("---\n");
    out.push_str(&note.content);
    out
}

fn yaml_string(value: &str) -> String {
    serde_json::to_string(value).expect("strings always serialize")
}

fn markdown_parts(raw: &str) -> Option<(&str, &str, &str)> {
    let start = raw.trim_start_matches('\u{feff}').trim_start();
    let (opening, rest) = start.split_once('\n')?;
    if opening.trim_end() != "---" {
        return None;
    }
    let closing = rest
        .split_inclusive('\n')
        .scan(0, |offset, line| {
            let start = *offset;
            *offset += line.len();
            Some((start, line))
        })
        .find(|(_, line)| line.trim_end() == "---")?;
    Some((
        &raw[..raw.len() - start.len()],
        &rest[..closing.0],
        &rest[closing.0 + closing.1.len()..],
    ))
}

fn rewrite_note(file: &NoteFile, note: &Note) -> Result<String, String> {
    let Some((prefix, header, _)) = markdown_parts(&file.raw) else {
        return Ok(serialize_note(note));
    };
    if header
        .lines()
        .all(|line| line.trim().is_empty() || line.trim_start().starts_with('#'))
    {
        let fields = serialize_note(note);
        return Ok(format!("{prefix}---\n{header}{}", &fields["---\n".len()..]));
    }
    let doc = match yaml_edit::Document::from_str(header) {
        Ok(doc) => doc,
        Err(_) if is_legacy_header(header) => {
            return Ok(format!("{prefix}{}", serialize_note(note)))
        }
        Err(err) => {
            return Err(format!(
                "无法保留 Markdown 属性 / Invalid Markdown frontmatter: {err}"
            ))
        }
    };
    let mapping = doc
        .as_mapping()
        .ok_or("Markdown frontmatter must be a mapping")?;
    if note.title != file.note.title {
        mapping.set("title", note.title.as_str());
    }
    if note.folder != file.note.folder {
        mapping.set("folder", note.folder.as_str());
    }
    if Uuid::try_parse(&note.id).is_ok() {
        if mapping
            .get("kivio_note_id")
            .and_then(|value| value.as_scalar().map(|scalar| scalar.as_string()))
            .as_deref()
            != Some(note.id.as_str())
        {
            mapping.set("kivio_note_id", note.id.as_str());
        }
    } else if mapping.get("kivio_note_id").is_some() {
        mapping.remove("kivio_note_id");
    }
    if mapping.get("created_at").is_none() {
        mapping.set("created_at", note.created_at.as_str());
    }
    mapping.set("updated_at", note.updated_at.as_str());
    let header = doc.to_string();
    let separator = if header.ends_with('\n') { "" } else { "\n" };
    Ok(format!(
        "{prefix}---\n{header}{separator}---\n{}",
        note.content
    ))
}

// Early Kivio versions wrote unescaped titles into an otherwise flat header.
fn is_legacy_header(header: &str) -> bool {
    header.contains("\ncreated_at:")
        && header.contains("\nupdated_at:")
        && header.lines().all(|line| {
            [
                "title:",
                "created_at:",
                "updated_at:",
                "kivio_note_id:",
                "folder:",
                "category:",
                "origin:",
            ]
            .iter()
            .any(|key| line.starts_with(key))
        })
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

/// 外部 Markdown 没有笔记 ID 时使用文件名，时间戳缺失时使用文件 mtime。
fn parse_note(stem: &str, raw: &str, fallback_updated: Option<DateTime<Utc>>) -> Note {
    let parts = markdown_parts(raw);
    let mapping = parts
        .and_then(|(_, header, _)| yaml_edit::Document::from_str(header).ok())
        .and_then(|doc| doc.as_mapping());
    let legacy = if parts.is_some() && mapping.is_none() {
        split_frontmatter(raw.trim_start_matches('\u{feff}')).0
    } else {
        HashMap::new()
    };
    let field = |key: &str| {
        mapping
            .as_ref()
            .and_then(|header| header.get(key))
            .and_then(|value| value.as_scalar().map(|scalar| scalar.as_string()))
            .or_else(|| legacy.get(key).cloned())
    };
    let id = field("kivio_note_id")
        .filter(|id| Uuid::try_parse(id).is_ok())
        .unwrap_or_else(|| stem.to_string());
    let body = parts.map_or_else(|| raw.trim_start_matches('\u{feff}'), |(_, _, body)| body);
    let fallback_ts = fallback_updated
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default();
    let title = field("title")
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| {
            if Uuid::try_parse(&id).is_ok() {
                String::new()
            } else {
                stem.to_string()
            }
        });
    let created_at = field("created_at")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| fallback_ts.clone());
    let updated_at = field("updated_at")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback_ts);
    // 兼容早期版本写过的 `category` 字段。
    let folder = field("folder")
        .or_else(|| field("category"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let origin = field("origin")
        .map(|s| normalize_origin(s.trim()))
        .unwrap_or_else(|| ORIGIN_USER.to_string());
    Note {
        id,
        title,
        content: body.to_string(),
        folder,
        origin,
        created_at,
        updated_at,
    }
}

fn read_note_file(path: &Path) -> Option<NoteFile> {
    let raw = std::fs::read_to_string(path).ok()?;
    let stem = path.file_stem()?.to_str()?;
    let mtime = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .map(DateTime::<Utc>::from);
    Some(NoteFile {
        note: parse_note(stem, &raw, mtime),
        raw,
    })
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
        let trimmed = text.trim_start_matches('\u{feff}').trim_start();
        let incomplete_header = "---".starts_with(trimmed)
            || trimmed.strip_prefix("---").is_some_and(|rest| {
                rest.starts_with(['\r', '\n']) && markdown_parts(&text).is_none()
            });
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
        Ok(self
            .entries(dir)?
            .into_iter()
            .map(|(_, meta)| meta)
            .collect())
    }

    fn entries(&self, dir: &Path) -> Result<Vec<(PathBuf, NoteMeta)>, String> {
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
        // Copied Markdown files can carry the same ID. Let the user address each copy by name.
        let mut seen = HashSet::new();
        let duplicates: HashSet<_> = next
            .values()
            .filter_map(|(_, _, meta)| (!seen.insert(&meta.id)).then_some(&meta.id))
            .collect();
        let mut metas: Vec<_> = next
            .iter()
            .map(|(path, (_, _, meta))| {
                let mut meta = meta.clone();
                if duplicates.contains(&meta.id) {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        meta.id = stem.to_string();
                    }
                }
                (path.clone(), meta)
            })
            .collect();
        *cached = next;
        metas.sort_by(|(_, a), (_, b)| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(metas)
    }

    fn read(&self, dir: &Path, id: &str) -> Result<(PathBuf, NoteFile), String> {
        validate_note_id(id)?;
        let path = dir.join(format!("{id}.md"));
        let named = match read_note_file(&path) {
            Some(file) if file.note.id == id => return Ok((path, file)),
            other => other,
        };
        if Uuid::try_parse(id).is_err() {
            let mut file = named.ok_or_else(|| format!("note not found: {id}"))?;
            file.note.id = id.to_string();
            return Ok((path, file));
        }
        let read_cached = || {
            let cached = self.0.lock().unwrap_or_else(|e| e.into_inner());
            let mut matches = cached.iter().filter(|(_, (_, _, meta))| meta.id == id);
            let (path, _) = matches.next()?;
            if matches.next().is_some() {
                return None;
            }
            let file = read_note_file(path).filter(|file| file.note.id == id)?;
            Some((path.clone(), file))
        };
        if let Some(found) = read_cached() {
            return Ok(found);
        }
        self.entries(dir)?;
        read_cached()
            .or_else(|| {
                named.map(|mut file| {
                    file.note.id = id.to_string();
                    (path, file)
                })
            })
            .ok_or_else(|| format!("note not found: {id}"))
    }

    fn search(&self, dir: &Path, query: &str) -> Result<Vec<NoteMeta>, String> {
        let query = query
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let mut hits = Vec::new();
        for (path, mut meta) in self.entries(dir)? {
            if let Some(file) = read_note_file(&path) {
                if let Some(excerpt) = search_excerpt(&file.note.content, &query) {
                    meta.preview = excerpt;
                    hits.push(meta);
                    continue;
                }
            }
            if meta.title.to_lowercase().contains(&query) {
                hits.push(meta);
            }
        }
        Ok(hits)
    }
}

fn search_excerpt(content: &str, query: &str) -> Option<String> {
    let compact = content.split_whitespace().collect::<Vec<_>>().join(" ");
    let offset = compact.to_lowercase().find(query)?;
    let mut folded_bytes = 0;
    let index = compact
        .chars()
        .take_while(|ch| {
            let before = folded_bytes;
            folded_bytes += ch.to_lowercase().map(char::len_utf8).sum::<usize>();
            before < offset && folded_bytes <= offset
        })
        .count();
    let start = index.saturating_sub(45);
    let mut chars = compact.chars().skip(start);
    let mut excerpt = String::new();
    if start > 0 {
        excerpt.push('…');
    }
    excerpt.extend(
        chars
            .by_ref()
            .take(PREVIEW_MAX_CHARS.max(query.chars().count() + index - start)),
    );
    if chars.next().is_some() {
        excerpt.push('…');
    }
    Some(excerpt)
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
    atomic_write(&path, &json, "note folders")
}

fn list_notes(app: AppHandle) -> Result<Vec<NoteMeta>, String> {
    app.state::<crate::state::AppState>()
        .note_catalog
        .list(&notes_dir(&app)?)
}

fn read_note(app: &AppHandle, id: &str) -> Result<(PathBuf, NoteFile), String> {
    app.state::<crate::state::AppState>()
        .note_catalog
        .read(&notes_dir(app)?, id)
}

/// 文件夹清单：folders.json 顺序在前，追加被笔记引用但清单里没有的（容错）。
fn list_folders(app: AppHandle) -> Result<Vec<String>, String> {
    let mut folders = read_folders_raw(&app);
    for note in list_notes(app.clone())? {
        let f = note.folder.trim();
        if !f.is_empty() && !folders.iter().any(|x| x == f) {
            folders.push(f.to_string());
        }
    }
    Ok(folders)
}

#[tauri::command]
pub async fn notes_folder_create(app: AppHandle, name: String) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _write = NOTE_WRITES.lock().unwrap_or_else(|e| e.into_inner());

        let name = sanitize_line(&name);
        if name.is_empty() {
            return Err("folder name empty".to_string());
        }
        let mut folders = read_folders_raw(&app);
        if !folders.iter().any(|x| x == &name) {
            folders.push(name);
            write_folders(&app, &folders)?;
        }
        list_folders(app)
    })
    .await
    .map_err(|error| format!("notes_folder_create failed: {error}"))?
}

#[tauri::command]
pub async fn notes_folder_rename(
    app: AppHandle,
    old: String,
    new: String,
) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _write = NOTE_WRITES.lock().unwrap_or_else(|e| e.into_inner());

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
        for meta in list_notes(app.clone())?
            .into_iter()
            .filter(|meta| meta.folder == old)
        {
            let (path, file) = read_note(&app, &meta.id)?;
            write_note_folder(&path, &file, &new)?;
        }
        list_folders(app)
    })
    .await
    .map_err(|error| format!("notes_folder_rename failed: {error}"))?
}

/// 删除文件夹：清单移除，其中的笔记回到库根（folder=""），不删笔记。
#[tauri::command]
pub async fn notes_folder_delete(app: AppHandle, name: String) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _write = NOTE_WRITES.lock().unwrap_or_else(|e| e.into_inner());

        let mut folders = read_folders_raw(&app);
        folders.retain(|x| x != &name);
        write_folders(&app, &folders)?;
        for meta in list_notes(app.clone())?
            .into_iter()
            .filter(|meta| meta.folder == name)
        {
            let (path, file) = read_note(&app, &meta.id)?;
            write_note_folder(&path, &file, "")?;
        }
        list_folders(app)
    })
    .await
    .map_err(|error| format!("notes_folder_delete failed: {error}"))?
}

#[tauri::command]
pub async fn notes_read(app: AppHandle, id: String) -> Result<Note, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _write = NOTE_WRITES.lock().unwrap_or_else(|e| e.into_inner());

        read_note(&app, &id).map(|(_, file)| file.note)
    })
    .await
    .map_err(|error| format!("notes_read failed: {error}"))?
}

#[tauri::command]
pub async fn notes_create(
    app: AppHandle,
    title: String,
    content: String,
    folder: String,
    origin: String,
) -> Result<Note, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _write = NOTE_WRITES.lock().unwrap_or_else(|e| e.into_inner());
        create_note(&notes_dir(&app)?, title, content, folder, origin)
    })
    .await
    .map_err(|error| format!("notes_create failed: {error}"))?
}

fn create_note(
    dir: &Path,
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
    let mut staged =
        tempfile::NamedTempFile::new_in(dir).map_err(|err| format!("stage note failed: {err}"))?;
    staged
        .write_all(serialize_note(&note).as_bytes())
        .and_then(|()| staged.as_file().sync_all())
        .map_err(|err| format!("write note failed: {err}"))?;
    publish_note(&staged.into_temp_path(), dir, &note_file_stem(&note.title))?;
    Ok(note)
}

fn apply_note_field(
    current: &mut String,
    edit: Option<NoteFieldEdit>,
    field: &'static str,
) -> Result<(), NoteWriteError> {
    if let Some(edit) = edit {
        if *current != edit.before && *current != edit.after {
            return Err(NoteWriteError::Conflict { field });
        }
        *current = edit.after;
    }
    Ok(())
}

fn save_note(path: &Path, file: &NoteFile, mut note: Note) -> Result<Note, NoteWriteError> {
    if note.title == file.note.title
        && note.content == file.note.content
        && note.folder == file.note.folder
    {
        return Ok(file.note.clone());
    }
    note.updated_at = now_iso();
    let raw = rewrite_note(file, &note)?;
    let legacy_path = path.file_stem().and_then(|s| s.to_str()) == Some(note.id.as_str());
    let rename = note.title != file.note.title
        && Uuid::try_parse(&note.id).is_ok()
        && (legacy_path || note_file_stem(&note.title) != note_file_stem(&file.note.title));
    if std::fs::read_to_string(path).map_err(|err| err.to_string())? != file.raw {
        return Err(NoteWriteError::Conflict { field: "file" });
    }
    let path = if rename {
        if legacy_path {
            // Keep the old ID readable if the write after a rename fails.
            atomic_write(path, &rewrite_note(file, &file.note)?, "note")?;
        }
        publish_note(
            path,
            path.parent().ok_or("missing note directory".to_string())?,
            &note_file_stem(&note.title),
        )?
    } else {
        path.to_owned()
    };
    atomic_write(&path, &raw, "note")?;
    Ok(note)
}

fn update_note(path: &Path, file: &NoteFile, mut edit: NoteEdit) -> Result<Note, NoteWriteError> {
    if let Some(title) = &mut edit.title {
        title.after = sanitize_line(&title.after);
    }
    if let Some(folder) = &mut edit.folder {
        folder.after = sanitize_line(&folder.after);
    }
    let mut note = file.note.clone();
    apply_note_field(&mut note.title, edit.title, "title")?;
    apply_note_field(&mut note.content, edit.content, "content")?;
    apply_note_field(&mut note.folder, edit.folder, "folder")?;
    save_note(path, file, note)
}

fn write_note_folder(path: &Path, file: &NoteFile, folder: &str) -> Result<(), String> {
    let mut note = file.note.clone();
    note.folder = sanitize_line(folder);
    save_note(path, file, note)
        .map(|_| ())
        .map_err(|error| match error {
            NoteWriteError::Conflict { .. } => {
                "笔记文件已被修改 / Note changed on disk".to_string()
            }
            NoteWriteError::Storage { message } => message,
        })
}

/// 更新：origin 保持不变（笔记来源不因编辑而变），folder 可改（移动文件夹）。
#[tauri::command]
pub async fn notes_update(
    app: AppHandle,
    id: String,
    edit: NoteEdit,
) -> Result<Note, NoteWriteError> {
    tauri::async_runtime::spawn_blocking(move || {
        let _write = NOTE_WRITES.lock().unwrap_or_else(|e| e.into_inner());

        let (path, file) = read_note(&app, &id)?;
        update_note(&path, &file, edit)
    })
    .await
    .map_err(|error| format!("notes_update failed: {error}"))?
}

const TRASH: &str = ".trash";

fn trash_note(dir: &Path, path: &Path) -> Result<(), String> {
    let name = path.file_name().ok_or("missing note filename")?;
    if !path
        .symlink_metadata()
        .map_err(|err| err.to_string())?
        .is_file()
    {
        return Err("笔记回收站只支持普通文件 / Trash requires a regular note file".to_string());
    }
    let item = dir.join(TRASH).join(Uuid::new_v4().to_string());
    std::fs::create_dir_all(&item).map_err(|err| format!("create note trash failed: {err}"))?;
    if let Err(err) = rename_no_replace(path, &item.join(name)) {
        let _ = std::fs::remove_dir(item);
        return Err(format!("move note to trash failed: {err}"));
    }
    Ok(())
}

fn trashed_file(item: &Path) -> Result<Option<PathBuf>, String> {
    let entries = std::fs::read_dir(item).map_err(|err| err.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|err| err.to_string())?;
        if entry.file_type().map_err(|err| err.to_string())?.is_file()
            && entry.path().extension().is_some_and(|ext| ext == "md")
        {
            return Ok(Some(entry.path()));
        }
    }
    Ok(None)
}

fn list_trashed_notes(dir: &Path) -> Result<Vec<NoteMeta>, String> {
    let entries = match std::fs::read_dir(dir.join(TRASH)) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err.to_string()),
    };
    let mut notes = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| err.to_string())?;
        let id = entry.file_name();
        let Some(id) = id.to_str().filter(|id| Uuid::try_parse(id).is_ok()) else {
            continue;
        };
        if !entry.file_type().map_err(|err| err.to_string())?.is_dir() {
            continue;
        }
        let Some(path) = trashed_file(&entry.path())? else {
            continue;
        };
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let file = std::fs::File::open(&path).map_err(|err| err.to_string())?;
        let mtime = file
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .map(DateTime::<Utc>::from);
        if let Some(mut note) = read_note_meta(file, stem, mtime) {
            note.id = id.to_string();
            notes.push(note);
        }
    }
    notes.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(notes)
}

fn restore_trashed_note(dir: &Path, id: Uuid) -> Result<(), String> {
    let item = dir.join(TRASH).join(id.to_string());
    if !item.symlink_metadata().is_ok_and(|meta| meta.is_dir()) {
        return Err("deleted note not found".to_string());
    }
    let path = trashed_file(&item)?.ok_or("deleted note not found")?;
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or("invalid note filename")?;
    publish_note(&path, dir, stem)?;
    let _ = std::fs::remove_dir(item);
    Ok(())
}

#[tauri::command]
pub async fn notes_delete(app: AppHandle, id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _write = NOTE_WRITES.lock().unwrap_or_else(|e| e.into_inner());

        let path = match read_note(&app, &id) {
            Ok((path, _)) => path,
            Err(error) => {
                validate_note_id(&id)?;
                let path = notes_dir(&app)?.join(format!("{id}.md"));
                if !path.is_file() || read_note_file(&path).is_some() {
                    return Err(error);
                }
                path
            }
        };
        trash_note(&notes_dir(&app)?, &path)
    })
    .await
    .map_err(|error| format!("notes_delete failed: {error}"))?
}

#[tauri::command]
pub async fn notes_trash_list(app: AppHandle) -> Result<Vec<NoteMeta>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _write = lock_storage();
        list_trashed_notes(&notes_dir(&app)?)
    })
    .await
    .map_err(|err| err.to_string())?
}

#[tauri::command]
pub async fn notes_restore(app: AppHandle, id: String) -> Result<(), String> {
    let id = Uuid::try_parse(&id).map_err(|_| "invalid deleted note ID")?;
    tauri::async_runtime::spawn_blocking(move || {
        let _write = lock_storage();
        restore_trashed_note(&notes_dir(&app)?, id)
    })
    .await
    .map_err(|err| err.to_string())?
}

#[tauri::command]
pub async fn notes_search(app: AppHandle, query: String) -> Result<Vec<NoteMeta>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<crate::state::AppState>()
            .note_catalog
            .search(&notes_dir(&app)?, &query)
    })
    .await
    .map_err(|err| err.to_string())?
}

/// 笔记目录的绝对路径。前端用它订阅 `workspace:activity`，这样用户往目录里拖进
/// 外部 `.md` 后列表自己刷新，不需要手动触发。
#[tauri::command]
pub fn notes_dir_path(app: AppHandle) -> Result<String, String> {
    Ok(notes_dir(&app)?.display().to_string())
}

/// 在系统文件管理器里打开笔记目录，让用户直接拖入外部 `.md`。
/// 拖进来的文件会被下一次 `notes_list` 读到，无需导入步骤。
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

    fn field(before: &str, after: &str) -> Option<NoteFieldEdit> {
        Some(NoteFieldEdit {
            before: before.into(),
            after: after.into(),
        })
    }

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
    fn creating_titled_notes_writes_readable_markdown_files_without_overwriting() {
        let dir = tempfile::tempdir().unwrap();
        let create = |content: &str| {
            create_note(
                dir.path(),
                "项目周报".into(),
                content.into(),
                String::new(),
                ORIGIN_CHAT.into(),
            )
            .unwrap()
        };
        let first = create("第一篇");
        let second = create("第二篇");
        for (name, original) in [("项目周报.md", first), ("项目周报 (2).md", second)] {
            let path = dir.path().join(name);
            assert!(path.is_file(), "expected a readable file name: {name}");
            let saved = read_note_file(&path).unwrap().note;
            assert_eq!(saved.id, original.id);
            assert_eq!(saved.content, original.content);
        }
    }

    #[test]
    fn saving_an_untitled_note_uses_its_title_and_keeps_its_id_after_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let note = create_note(
            dir.path(),
            String::new(),
            String::new(),
            "工作".into(),
            ORIGIN_USER.into(),
        )
        .unwrap();
        let original_id = note.id;
        let catalog = NoteCatalog::default();
        let (path, note) = catalog.read(dir.path(), &original_id).unwrap();
        assert_eq!(path, dir.path().join("无标题笔记.md"));

        let occupied = dir.path().join("项目-复盘.md");
        std::fs::write(&occupied, "已有外部笔记").unwrap();
        update_note(
            &path,
            &note,
            NoteEdit {
                title: field("", "项目/复盘"),
                content: field("", "正文"),
                ..NoteEdit::default()
            },
        )
        .unwrap();

        let reopened = NoteCatalog::default();
        let (path, file) = reopened.read(dir.path(), &original_id).unwrap();
        let note = file.note;
        assert_eq!(path, dir.path().join("项目-复盘 (2).md"));
        assert_eq!(note.id, original_id);
        assert_eq!(note.title, "项目/复盘");
        assert_eq!(note.content, "正文");
        assert_eq!(note.folder, "工作");
        assert!(!dir.path().join("无标题笔记.md").exists());
        assert_eq!(std::fs::read_to_string(occupied).unwrap(), "已有外部笔记");
        assert_eq!(reopened.list(dir.path()).unwrap().len(), 2);
    }

    #[test]
    fn editing_a_legacy_uuid_note_keeps_its_original_id() {
        let dir = tempfile::tempdir().unwrap();
        let id = Uuid::new_v4().to_string();
        let legacy_path = dir.path().join(format!("{id}.md"));
        std::fs::write(&legacy_path, serialize_note(&sample("", ORIGIN_CHAT))).unwrap();

        let (path, note) = NoteCatalog::default().read(dir.path(), &id).unwrap();
        assert_eq!(note.note.id, id);
        update_note(
            &path,
            &note,
            NoteEdit {
                title: field("标题", "NUL.txt"),
                ..NoteEdit::default()
            },
        )
        .unwrap();

        let (path, note) = NoteCatalog::default().read(dir.path(), &id).unwrap();
        assert_eq!(path, dir.path().join("笔记 - NUL.txt.md"));
        assert_eq!(note.note.id, id);
        assert_eq!(note.note.origin, ORIGIN_CHAT);
        assert!(!legacy_path.exists());
    }

    #[test]
    fn copied_markdown_notes_can_be_opened_independently() {
        let dir = tempfile::tempdir().unwrap();
        create_note(
            dir.path(),
            "周报".into(),
            "正文".into(),
            String::new(),
            ORIGIN_USER.into(),
        )
        .unwrap();
        let copy = dir.path().join("周报副本.md");
        std::fs::copy(dir.path().join("周报.md"), &copy).unwrap();

        let catalog = NoteCatalog::default();
        let metas = catalog.list(dir.path()).unwrap();
        assert_eq!(metas.len(), 2);
        assert_ne!(metas[0].id, metas[1].id);
        let (path, note) = catalog.read(dir.path(), "周报副本").unwrap();
        assert_eq!(path, copy);
        assert_eq!(note.note.content, "正文");
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
        assert!(raw.contains("folder: \"工作\""));
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

    #[test]
    fn saving_and_moving_external_markdown_preserves_properties_and_independent_edits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("周报.md");
        let untouched = "tags:\n  - 工作 # keep this comment\naliases: [weekly]\ncategory: 工作\n";
        std::fs::write(&path, format!("---\n{untouched}---\noriginal")).unwrap();
        let catalog = NoteCatalog::default();
        let (_, opened) = catalog.read(dir.path(), "周报").unwrap();
        assert_eq!(opened.note.title, "周报");
        std::fs::write(&path, format!("---\n{untouched}---\nexternal change")).unwrap();
        let (_, current) = catalog.read(dir.path(), "周报").unwrap();
        let saved = update_note(
            &path,
            &current,
            NoteEdit {
                title: field(&opened.note.title, "项目: 周报"),
                ..NoteEdit::default()
            },
        )
        .unwrap();
        assert_eq!(saved.content, "external change");
        let (_, current) = catalog.read(dir.path(), "周报").unwrap();
        let error = update_note(
            &path,
            &current,
            NoteEdit {
                content: field(&opened.note.content, "local change"),
                ..NoteEdit::default()
            },
        )
        .unwrap_err();
        assert!(matches!(
            error,
            NoteWriteError::Conflict { field: "content" }
        ));
        write_note_folder(&path, &current, "").unwrap();
        let (_, reopened) = catalog.read(dir.path(), "周报").unwrap();
        assert!(reopened.raw.contains(untouched));
        assert_eq!(reopened.note.title, "项目: 周报");
        assert_eq!(reopened.note.content, "external change");
        assert_eq!(reopened.note.folder, "");
    }

    #[test]
    fn full_text_search_finds_unicode_text_past_the_preview_and_keeps_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let body = format!("{}关键线索 and CafÉ", "开头 ".repeat(100));
        create_note(
            dir.path(),
            "资料".into(),
            body,
            "工作".into(),
            ORIGIN_USER.into(),
        )
        .unwrap();
        let catalog = NoteCatalog::default();
        assert!(!catalog.list(dir.path()).unwrap()[0]
            .preview
            .contains("关键线索"));
        let notes = catalog.search(dir.path(), "关键线索 AND CAFÉ").unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].folder, "工作");
        assert!(notes[0].preview.contains("关键线索 and CafÉ"));
    }

    #[test]
    fn restore_keeps_original_markdown_and_never_overwrites_an_occupied_filename() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("工作.md");
        let original = "---\ntags: [team]\n---\n原始正文";
        std::fs::write(&path, original).unwrap();
        trash_note(dir.path(), &path).unwrap();
        assert!(!path.exists());
        assert!(NoteCatalog::default().list(dir.path()).unwrap().is_empty());
        let trash = list_trashed_notes(dir.path()).unwrap();
        assert_eq!(trash[0].title, "工作");
        std::fs::write(&path, "new note").unwrap();
        restore_trashed_note(dir.path(), Uuid::parse_str(&trash[0].id).unwrap()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new note");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("工作 (2).md")).unwrap(),
            original
        );
        assert!(list_trashed_notes(dir.path()).unwrap().is_empty());
    }
}

#[tauri::command]
pub async fn notes_list(app: AppHandle) -> Result<Vec<NoteMeta>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _write = NOTE_WRITES.lock().unwrap_or_else(|e| e.into_inner());
        list_notes(app)
    })
    .await
    .map_err(|error| format!("notes_list failed: {error}"))?
}
#[tauri::command]
pub async fn notes_folders_list(app: AppHandle) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _write = NOTE_WRITES.lock().unwrap_or_else(|e| e.into_inner());
        list_folders(app)
    })
    .await
    .map_err(|error| format!("notes_folders_list failed: {error}"))?
}
