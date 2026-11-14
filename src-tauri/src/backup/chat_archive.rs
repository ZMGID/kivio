use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

const DIRECTORIES: [&str; 4] = [
    "conversations",
    "external-agent-sessions",
    "subagents",
    "artifacts",
];
const FORMAT: &str = "kivio-chat-backup";
pub(super) const MAX_BYTES: u64 = 50 * 1024 * 1024 * 1024;

fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn sibling(root: &Path, suffix: &str) -> PathBuf {
    root.with_file_name(format!(
        "{}.{suffix}",
        root.file_name().unwrap_or_default().to_string_lossy()
    ))
}
fn stage_path(root: &Path) -> PathBuf {
    sibling(root, "chat-restore")
}

pub(crate) fn export(root: &Path, destination: &Path) -> Result<(), String> {
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(error)?;
    let mut zip = zip::ZipWriter::new(temporary.as_file_mut());
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("manifest.json", options).map_err(error)?;
    serde_json::to_writer(
        &mut zip,
        &serde_json::json!({"type": FORMAT, "version": 1, "sourceRoot": root,
            "appVersion": option_env!("CARGO_PKG_VERSION").unwrap_or("development"),
            "createdAt": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(error)?.as_secs()}),
    )
    .map_err(error)?;
    fn append(
        zip: &mut zip::ZipWriter<&mut fs::File>,
        root: &Path,
        path: &Path,
        remaining: &mut u64,
    ) -> Result<(), String> {
        let metadata = fs::symlink_metadata(path).map_err(error)?;
        if metadata.file_type().is_symlink() {
            return Err(format!("不能备份符号链接：{}", path.display()));
        }
        if metadata.is_dir() {
            for entry in fs::read_dir(path).map_err(error)? {
                append(zip, root, &entry.map_err(error)?.path(), remaining)?;
            }
        } else if metadata.is_file() {
            let name = path
                .strip_prefix(root)
                .map_err(error)?
                .to_str()
                .ok_or("备份路径不是 UTF-8")?
                .replace('\\', "/");
            zip.start_file(
                name,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )
            .map_err(error)?;
            let copied = std::io::copy(
                &mut fs::File::open(path).map_err(error)?.take(*remaining + 1),
                zip,
            )
            .map_err(error)?;
            *remaining = remaining.checked_sub(copied).ok_or("聊天数据超过 50 GiB")?;
        } else {
            return Err("不支持的聊天文件类型".into());
        }
        Ok(())
    }
    let target = parent
        .canonicalize()
        .map_err(error)?
        .join(destination.file_name().ok_or("无效备份路径")?);
    let mut remaining = MAX_BYTES;
    for directory in DIRECTORIES {
        let path = root.join(directory);
        if path.exists() {
            if target.starts_with(path.canonicalize().map_err(error)?) {
                return Err("请将备份保存到聊天数据目录之外".into());
            }
            append(&mut zip, root, &path, &mut remaining)?;
        }
    }
    zip.finish().map_err(error)?;
    temporary.as_file().sync_all().map_err(error)?;
    temporary.persist(destination).map_err(error)?;
    Ok(())
}

fn remap(value: &mut serde_json::Value, source: &str, target: &str) {
    match value {
        serde_json::Value::String(text) => {
            if let Some(tail) = text
                .strip_prefix(source)
                .filter(|tail| tail.starts_with(['/', '\\']))
            {
                // Only app-owned resource paths move; project working directories stay unchanged.
                if DIRECTORIES.iter().any(|dir| {
                    tail.trim_start_matches(['/', '\\'])
                        .starts_with(&format!("{dir}/"))
                        || tail
                            .trim_start_matches(['/', '\\'])
                            .starts_with(&format!("{dir}\\"))
                }) {
                    *text = format!("{target}{}", tail.replace('\\', "/"));
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                remap(item, source, target);
            }
        }
        serde_json::Value::Object(items) => {
            for item in items.values_mut() {
                remap(item, source, target);
            }
        }
        _ => {}
    }
}

pub(crate) fn prepare(root: &Path, archive: &Path) -> Result<(), String> {
    let stage = stage_path(root);
    if stage.exists() {
        return Err("已有待恢复的聊天备份，请先重启应用".into());
    }
    let mut zip = zip::ZipArchive::new(fs::File::open(archive).map_err(error)?).map_err(error)?;
    let manifest: serde_json::Value = serde_json::from_reader(
        zip.by_name("manifest.json")
            .map_err(error)?
            .take(1024 * 1024),
    )
    .map_err(error)?;
    if manifest["type"] != FORMAT || manifest["version"] != 1 {
        return Err("不支持的聊天备份格式或版本".into());
    }
    let source = manifest["sourceRoot"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("备份缺少原始数据路径")?;
    let temporary = tempfile::tempdir_in(root.parent().ok_or("无效数据目录")?).map_err(error)?;
    let data = temporary.path().join("data");
    for directory in DIRECTORIES {
        fs::create_dir_all(data.join(directory)).map_err(error)?;
    }
    let mut names = HashSet::new();
    let mut remaining = MAX_BYTES;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(error)?;
        let name = entry.name().map_err(error)?.into_owned();
        if name == "manifest.json" {
            continue;
        }
        let path = Path::new(&name);
        if name.contains(['\\', ':'])
            || name
                .split('/')
                .any(|part| part == "." || part == ".." || part.ends_with(['.', ' ']))
            || !path
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
            || !DIRECTORIES
                .iter()
                .any(|dir| name.starts_with(&format!("{dir}/")))
            || !names.insert(name.to_lowercase())
            || entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(format!("无效或重复的备份路径：{name}"));
        }
        let destination = data.join(path);
        if entry.is_dir() {
            fs::create_dir_all(destination).map_err(error)?;
            continue;
        }
        fs::create_dir_all(destination.parent().ok_or("无效路径")?).map_err(error)?;
        let mut file = fs::File::create(&destination).map_err(error)?;
        let copied =
            std::io::copy(&mut (&mut entry).take(remaining + 1), &mut file).map_err(error)?;
        remaining = remaining
            .checked_sub(copied)
            .ok_or("聊天备份解压后超过 50 GiB")?;
        file.sync_all().map_err(error)?;
        drop(file);
        if path.components().count() == 2
            && (name.starts_with("conversations/") || name.starts_with("external-agent-sessions/"))
            && path.extension().is_some_and(|ext| ext == "json")
        {
            let mut value: serde_json::Value =
                serde_json::from_reader(fs::File::open(&destination).map_err(error)?)
                    .map_err(error)?;
            if source != root.to_string_lossy() {
                remap(&mut value, source, &root.to_string_lossy());
                fs::write(&destination, serde_json::to_vec(&value).map_err(error)?)
                    .map_err(error)?;
            }
        }
        if name.starts_with("conversations/")
            && path.components().count() == 2
            && name.ends_with(".draft.jsonl")
            && source != root.to_string_lossy()
        {
            let raw = fs::read_to_string(&destination).map_err(error)?;
            let mut output = String::new();
            for line in raw.lines() {
                if let Ok(mut value) = serde_json::from_str::<serde_json::Value>(line) {
                    remap(&mut value, source, &root.to_string_lossy());
                    output.push_str(&serde_json::to_string(&value).map_err(error)?);
                } else {
                    output.push_str(line);
                }
                output.push('\n');
            }
            fs::write(&destination, output).map_err(error)?;
        }
        fs::OpenOptions::new()
            .write(true)
            .open(&destination)
            .map_err(error)?
            .sync_all()
            .map_err(error)?;
    }
    fs::write(temporary.path().join("ready"), FORMAT).map_err(error)?;
    fs::OpenOptions::new()
        .write(true)
        .open(temporary.path().join("ready"))
        .map_err(error)?
        .sync_all()
        .map_err(error)?;
    fs::rename(temporary.path(), &stage).map_err(error)?;
    Ok(())
}

/// Before stores open, resume interrupted directory switches. Previous data is retained.
pub(crate) fn restore_at_startup(root: &Path) -> Result<(), String> {
    let stage = stage_path(root);
    if !stage.join("ready").exists() {
        return Ok(());
    }
    let previous = stage.join("previous");
    fs::create_dir_all(&previous).map_err(error)?;
    fs::create_dir_all(root).map_err(error)?;
    for directory in DIRECTORIES {
        let incoming = stage.join("data").join(directory);
        if !incoming.exists() {
            continue;
        }
        let current = root.join(directory);
        let old = previous.join(directory);
        if !old.exists() {
            if current.exists() {
                fs::rename(&current, &old).map_err(error)?;
            } else {
                fs::create_dir(&old).map_err(error)?;
            }
        }
        fs::rename(incoming, current).map_err(error)?;
    }
    let retained = sibling(
        root,
        &format!("chat-before-restore-{}", uuid::Uuid::new_v4()),
    );
    fs::rename(stage, retained).map_err(error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn restart_resumes_after_old_directory_was_moved() {
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("source");
        fs::create_dir_all(source.join("conversations")).unwrap();
        fs::write(source.join("conversations/conv_new.json"), b"{}").unwrap();
        let archive = workspace.path().join("chat.zip");
        export(&source, &archive).unwrap();
        let target = workspace.path().join("target.dev");
        fs::create_dir_all(target.join("conversations")).unwrap();
        fs::write(target.join("conversations/conv_old.json"), b"{}").unwrap();
        prepare(&target, &archive).unwrap();
        let stage = stage_path(&target);
        fs::create_dir(stage.join("previous")).unwrap();
        fs::rename(
            target.join("conversations"),
            stage.join("previous/conversations"),
        )
        .unwrap();
        restore_at_startup(&target).unwrap();
        assert!(target.join("conversations/conv_new.json").exists());
        assert!(fs::read_dir(workspace.path())
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with("target.dev.chat-before-restore-")
                && entry
                    .path()
                    .join("previous/conversations/conv_old.json")
                    .exists()));
        assert_ne!(
            stage_path(&target),
            stage_path(&workspace.path().join("target.test"))
        );
    }

    #[test]
    fn failed_export_preserves_previous_archive_and_rejects_self_inclusion() {
        let workspace = tempfile::tempdir().unwrap();
        fs::create_dir(workspace.path().join("conversations")).unwrap();
        let path = workspace.path().join("conversations/backup.zip");
        fs::write(&path, b"previous").unwrap();
        assert!(export(workspace.path(), &path).is_err());
        assert_eq!(fs::read(path).unwrap(), b"previous");
    }
    #[test]
    fn round_trip_keeps_project_order_messages_and_binary_attachments() {
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("source");
        fs::create_dir_all(source.join("conversations/conv_one_attachments")).unwrap();
        fs::write(
            source.join("conversations/projects.json"),
            br#"{"projects":[{"id":"b","path":"D:/code/demo"},{"id":"a"}]}"#,
        )
        .unwrap();
        fs::write(source.join("conversations/conv_one.json"), serde_json::to_vec(&serde_json::json!({"projectId":"b","messages":[{"content":"hello","path":source.join("conversations/conv_one_attachments/image.png") }]})).unwrap()).unwrap();
        fs::write(
            source.join("conversations/conv_one_attachments/image.png"),
            [0, 1, 2, 255],
        )
        .unwrap();
        fs::write(
            source.join("conversations/conv_one_attachments/raw.json"),
            b"not parsed: user attachment",
        )
        .unwrap();
        fs::write(source.join("settings.json"), b"private configuration").unwrap();
        let archive = workspace.path().join("chat.zip");
        export(&source, &archive).unwrap();
        let target = workspace.path().join("target");
        fs::create_dir_all(target.join("conversations")).unwrap();
        fs::write(target.join("conversations/old.json"), b"{}").unwrap();
        prepare(&target, &archive).unwrap();
        assert!(target.join("conversations/old.json").exists());
        restore_at_startup(&target).unwrap();
        restore_at_startup(&target).unwrap();
        assert!(!target.join("conversations/old.json").exists());
        assert!(!target.join("settings.json").exists());
        assert_eq!(
            fs::read(target.join("conversations/conv_one_attachments/raw.json")).unwrap(),
            b"not parsed: user attachment"
        );
        assert_eq!(
            fs::read(target.join("conversations/conv_one_attachments/image.png")).unwrap(),
            [0, 1, 2, 255]
        );
        let restored: serde_json::Value =
            serde_json::from_slice(&fs::read(target.join("conversations/conv_one.json")).unwrap())
                .unwrap();
        assert!(restored["messages"][0]["path"]
            .as_str()
            .unwrap()
            .starts_with(&*target.to_string_lossy()));
        assert_eq!(
            fs::read(target.join("conversations/projects.json")).unwrap(),
            fs::read(source.join("conversations/projects.json")).unwrap()
        );
    }
    #[test]
    fn rejects_traversal_without_touching_live_data() {
        let workspace = tempfile::tempdir().unwrap();
        let path = workspace.path().join("bad.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("manifest.json", options).unwrap();
        zip.write_all(br#"{"type":"kivio-chat-backup","version":1,"sourceRoot":"/old"}"#)
            .unwrap();
        zip.start_file("conversations/../../escape", options)
            .unwrap();
        zip.write_all(b"no").unwrap();
        zip.finish().unwrap();
        let root = workspace.path().join("target");
        assert!(prepare(&root, &path).is_err());
        assert!(!root.exists());
        assert!(!stage_path(&root).exists());
    }
}
