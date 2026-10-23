//! Conversation-owned attachment copy and cleanup, shared by desktop hosts.
//! Blocking I/O; callers hold the conversation write/run reservation.
use std::{collections::HashSet, fs, path::Path};

/// GC 自动生成的附件文件前缀。**只清这两类**：
/// - `msgimg-` 模型看过的图（`attachments::externalize_model_message_images`）
/// - `artifact-` 工具产出的图（`attachments::externalize_artifact`）
///
/// 用户上传的附件是 `att_<uuid>-<原名>`，**永不自动删**：即使当前没有任何消息引用它
/// （用户删了那条消息），那也是用户自己拖进来的文件，删掉是数据丢失。
const GC_MANAGED_ATTACHMENT_PREFIXES: &[&str] = &["msgimg-", "artifact-"];

fn is_gc_managed_attachment(name: &str) -> bool {
    GC_MANAGED_ATTACHMENT_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

/// 收集一个会话附件目录里**没有任何消息引用**的 GC 托管附件。
///
/// 只读对账（附件目录 + 目录里的文件名 + 消息引用路径），便于单测覆盖那条
/// 关键不变式：**有效引用集合为空时必须返回空**——若上层因为任何原因（消息还没落盘、
/// 转录被剥离、解析失败）交来一个空引用集，绝不能把整目录判成孤儿全删。
pub fn unreferenced_attachment_names(
    attachment_dir: &Path,
    files_on_disk: &[String],
    referenced: &HashSet<String>,
) -> Vec<String> {
    let referenced: HashSet<&str> = referenced
        .iter()
        .filter_map(|path| attachment_reference_name(attachment_dir, path))
        .collect();
    if referenced.is_empty() {
        // 保守：一个都不删。宁可留垃圾，也不能误删还在用的图。
        return Vec::new();
    }
    files_on_disk
        .iter()
        .filter(|name| is_gc_managed_attachment(name) && !referenced.contains(name.as_str()))
        .cloned()
        .collect()
}

/// 裸文件名与当前附件目录的绝对路径使用同一对账规则。外部路径、子目录和
/// `..` 不变成文件名：它们不能充当可信引用，让原本应保守跳过的 GC 开始删除。
/// 使用平台 Path 组件，因此 Windows 的盘符、UNC 与两种分隔符也遵守目录边界。
fn attachment_reference_name<'a>(attachment_dir: &Path, reference: &'a str) -> Option<&'a str> {
    let path = Path::new(reference);
    let relative = if path.is_absolute() {
        path.strip_prefix(attachment_dir).ok()?
    } else {
        path
    };
    let mut components = relative
        .components()
        .filter(|component| *component != std::path::Component::CurDir);
    let std::path::Component::Normal(name) = components.next()? else {
        return None;
    };
    if components.next().is_some() {
        return None;
    }
    let name = name.to_str()?;
    // On Unix a Windows path is otherwise a single component. Never treat it as a bare name.
    if name.contains('\\') || name.contains(':') {
        return None;
    }
    Some(name)
}

/// 一条会话里被引用到的附件路径：消息附件、artifact 的 `path`、`model_messages` 里
/// 媒体部件的 `path`，以及压缩快照的媒体引用。对账时统一解析裸文件名与托管绝对路径。
pub fn referenced_attachment_names(
    conversation: &crate::chat::conversation_data::Conversation,
) -> HashSet<String> {
    let mut referenced = HashSet::new();
    collect_message_attachment_names(&conversation.messages, &mut referenced);
    if let Some(replay) = conversation
        .context_state
        .summary
        .as_ref()
        .and_then(|summary| summary.replay.as_ref())
    {
        for message in &replay.messages {
            collect_attachment_uri_names(message, &mut referenced);
        }
    }
    referenced
}

fn collect_message_attachment_names(
    messages: &[crate::chat::conversation_data::ChatMessage],
    referenced: &mut HashSet<String>,
) {
    for message in messages {
        for attachment in &message.attachments {
            if !attachment.path.is_empty() {
                referenced.insert(attachment.path.clone());
            }
        }
        let mut push_artifact_path =
            |artifact: &crate::chat::conversation_data::ChatToolArtifact| {
                if let Some(path) = artifact.path.as_deref().filter(|p| !p.is_empty()) {
                    referenced.insert(path.to_string());
                }
            };
        for artifact in &message.artifacts {
            push_artifact_path(artifact);
        }
        for tool_call in &message.tool_calls {
            for artifact in &tool_call.artifacts {
                push_artifact_path(artifact);
            }
        }
        for model_message in &message.model_messages {
            for part in &model_message.content {
                match part {
                    crate::chat::model::MessagePart::Image { path, .. }
                    | crate::chat::model::MessagePart::Video { path, .. } => {
                        if let Some(path) = path.as_deref().filter(|p| !p.is_empty()) {
                            referenced.insert(path.to_string());
                        }
                    }
                    // 工具结果里的 artifact 也带 path（与 `tool_calls[].artifacts` 可能不同源）。
                    crate::chat::model::MessagePart::ToolResult { artifacts, .. } => {
                        for artifact in artifacts {
                            if let Some(path) = artifact.path.as_deref().filter(|p| !p.is_empty()) {
                                referenced.insert(path.to_string());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        // 中断草稿的 `api_messages` 里图片被外置成 `kivio-attachment://<mime>/<文件名>`
        // 哨兵（`attachments::externalize_api_message_images`）。**这也是一处引用来源**，
        // 漏掉就会把中断草稿还要用的图当成孤儿删掉。
        for api_message in &message.api_messages {
            collect_attachment_uri_names(api_message, referenced);
        }
    }
}

/// 从一条 wire 消息里挖出所有 `kivio-attachment://` 哨兵引用的文件名。
///
/// 不复用 `attachments::api_message_image_url_slots`（那个要 `&mut`），这里是纯只读遍历，
/// 且刻意**不限定部件 type**：宁可多认几个槽位，也不要漏掉一个引用而误删文件。
fn collect_attachment_uri_names(value: &serde_json::Value, out: &mut HashSet<String>) {
    match value {
        serde_json::Value::String(text) => {
            if let Some(name) = attachment_uri_file_name(text) {
                out.insert(name.to_string());
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_attachment_uri_names(item, out);
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values() {
                collect_attachment_uri_names(item, out);
            }
        }
        _ => {}
    }
}

fn attachment_uri_file_name(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("kivio-attachment://")?;
    let (_mime, file_name) = rest.rsplit_once('/')?;
    if file_name.is_empty() || Path::new(file_name).components().count() != 1 {
        return None;
    }
    Some(file_name)
}

/// Read the persisted canonical transcript before collecting generated artifacts.
/// Uploaded `att_*` files and user workspaces are never removed here.
pub fn sweep_conversation_attachments_in_dir(
    conversations: &Path,
    conversation_id: &str,
) -> (usize, u64) {
    if !conversation_id.starts_with("conv_")
        || !conversation_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return (0, 0);
    }
    let dir = conversations.join(format!("{conversation_id}_attachments"));
    if !dir.is_dir() {
        return (0, 0);
    }
    // 读盘失败 ⇒ 引用集合不可信 ⇒ 一个都不删。
    let Ok(bytes) = fs::read(conversations.join(format!("{conversation_id}.json"))) else {
        return (0, 0);
    };
    let Ok(conversation) =
        serde_json::from_slice::<crate::chat::conversation_data::Conversation>(&bytes)
    else {
        return (0, 0);
    };

    let referenced = referenced_attachment_names(&conversation);
    let files_on_disk: Vec<String> = match fs::read_dir(&dir) {
        Ok(entries) => entries
            .flatten()
            .filter(|entry| entry.path().is_file())
            .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
            .collect(),
        Err(_) => return (0, 0),
    };
    let mut removed = 0usize;
    let mut freed = 0u64;
    for name in unreferenced_attachment_names(&dir, &files_on_disk, &referenced) {
        let path = dir.join(&name);
        let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if fs::remove_file(&path).is_ok() {
            removed += 1;
            freed += size;
        }
    }
    (removed, freed)
}

/// Deep-copy referenced bare filenames; workbench output and external folders stay put.
pub fn copy_forked_conversation_files_in_dirs(
    src_dir: &Path,
    dst_dir: &Path,
    messages: &mut [crate::chat::conversation_data::ChatMessage],
) -> Vec<String> {
    let mut referenced = HashSet::new();
    collect_message_attachment_names(messages, &mut referenced);
    let names: HashSet<&str> = referenced
        .iter()
        .filter_map(|path| attachment_reference_name(src_dir, path))
        .collect();
    if names.is_empty() {
        return Vec::new();
    }
    if let Err(error) = fs::create_dir_all(dst_dir) {
        return vec![format!("无法创建分支附件目录：{error}")];
    }
    let mut warnings = Vec::new();
    for name in names {
        let src = src_dir.join(name);
        let dst = dst_dir.join(name);
        if !src.is_file() {
            warnings.push(format!("源附件不存在：{name}"));
            continue;
        }
        if let Err(error) = fs::copy(&src, &dst) {
            warnings.push(format!("拷贝附件失败 {name}: {error}"));
        }
    }
    // Relative references keep their names. Absolute references owned by the
    // source conversation must point at the branch, never back at deletable history.
    let rebase = |path: &mut String| {
        if Path::new(path).is_absolute() {
            if let Some(name) = attachment_reference_name(src_dir, path) {
                *path = dst_dir.join(name).to_string_lossy().into_owned();
            }
        }
    };
    let artifact = |artifact: &mut crate::chat::conversation_data::ChatToolArtifact| {
        if let Some(path) = &mut artifact.path {
            rebase(path);
        }
    };
    for message in messages {
        for attachment in &mut message.attachments {
            rebase(&mut attachment.path);
        }
        for item in &mut message.artifacts {
            artifact(item);
        }
        for call in &mut message.tool_calls {
            for item in &mut call.artifacts {
                artifact(item);
            }
        }
        for model in &mut message.model_messages {
            for part in &mut model.content {
                match part {
                    crate::chat::model::MessagePart::Image {
                        path: Some(path), ..
                    }
                    | crate::chat::model::MessagePart::Video {
                        path: Some(path), ..
                    } => rebase(path),
                    crate::chat::model::MessagePart::ToolResult { artifacts, .. } => {
                        for item in artifacts {
                            artifact(item);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;
    // A branch must own all replay media, including media absent from the visible attachment row.
    #[test]
    fn branch_media_survives_source_deletion_without_copying_external_files() {
        use crate::chat::conversation_data::{ChatMessage, MessagePart, ModelMessage, ModelRole};
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let branch = root.path().join("branch");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("msgimg-hidden.png"), b"canonical media").unwrap();
        fs::write(source.join("msgimg-wire.png"), b"legacy media").unwrap();
        let external = root.path().join("external.png");
        fs::write(&external, b"external file").unwrap();
        let mut message: ChatMessage = serde_json::from_value(serde_json::json!({
            "id":"msg_answer","role":"assistant","content":"answer","timestamp":1,
            "api_messages":[{"role":"assistant","image_url":"kivio-attachment://image/png/msgimg-wire.png"}]
        })).unwrap();
        message.model_messages = vec![ModelMessage {
            role: ModelRole::Assistant,
            content: vec![
                MessagePart::Image {
                    mime_type: "image/png".into(),
                    data: String::new(),
                    path: Some(source.join("msgimg-hidden.png").to_string_lossy().into()),
                    detail: None,
                },
                MessagePart::Image {
                    mime_type: "image/png".into(),
                    data: String::new(),
                    path: Some(external.to_string_lossy().into()),
                    detail: None,
                },
            ],
        }];
        let source_message = serde_json::to_value(&message).unwrap();
        let mut messages = vec![message.clone()];
        assert!(copy_forked_conversation_files_in_dirs(&source, &branch, &mut messages).is_empty());
        fs::remove_dir_all(&source).unwrap();
        assert_eq!(
            fs::read(branch.join("msgimg-hidden.png")).unwrap(),
            b"canonical media"
        );
        assert_eq!(
            fs::read(branch.join("msgimg-wire.png")).unwrap(),
            b"legacy media"
        );
        let MessagePart::Image {
            path: Some(path), ..
        } = &messages[0].model_messages[0].content[0]
        else {
            panic!("missing replay image")
        };
        assert_eq!(Path::new(path), branch.join("msgimg-hidden.png"));
        assert!(!branch.join("external.png").exists());
        assert_eq!(fs::read(external).unwrap(), b"external file");
        assert_eq!(serde_json::to_value(message).unwrap(), source_message);
    }

    /// 中断草稿的 `api_messages` 哨兵也是一处引用来源，漏掉就会误删它还要用的图。
    #[test]
    fn attachment_uri_names_are_collected_from_wire_messages() {
        let api_message = serde_json::json!({
            "role": "user",
            "content": [
                { "type": "text", "text": "看图" },
                {
                    "type": "image_url",
                    "image_url": { "url": "kivio-attachment://image/png/msgimg-wire.png" }
                },
                {
                    "type": "input_image",
                    "image_url": "kivio-attachment://image/jpeg/msgimg-responses.jpg"
                }
            ]
        });
        let mut names = HashSet::new();
        collect_attachment_uri_names(&api_message, &mut names);

        let mut sorted: Vec<_> = names.into_iter().collect();
        sorted.sort();
        assert_eq!(sorted, vec!["msgimg-responses.jpg", "msgimg-wire.png"]);
    }

    /// 路径穿越的哨兵不进引用集合（它本来也不该存在），且不 panic。
    #[test]
    fn attachment_uri_names_reject_traversal() {
        assert_eq!(
            attachment_uri_file_name("kivio-attachment://image/png/msgimg-ok.png"),
            Some("msgimg-ok.png")
        );
        assert!(attachment_uri_file_name("kivio-attachment://image/png/").is_none());
        assert!(attachment_uri_file_name("data:image/png;base64,AAAA").is_none());
    }
}
