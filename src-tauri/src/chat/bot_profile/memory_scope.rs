//! Private Bot memory. Global L1/L2 stays where it is and is not injected.

use std::path::Path;

use crate::chat::memory::{self, MemoryLayer};

use super::types::{SharedMemoryNote, MAX_SHARED_NOTES, MAX_SHARED_NOTE_CHARS};

const GLOBAL_INJECTION: &str = "disabled";
const DISABLED_WARNING: &str =
    "全局记忆开关已关闭，这份 Bot 的私有记忆本轮不注入，也没有改用全局记忆。";

pub fn global_injection_label() -> &'static str {
    GLOBAL_INJECTION
}

/// Private L1 and explicit shared notes only.
/// The primary coordinating role is not written here. `reply.rs` reads `runtime_scope.primary`.
pub fn private_memory_prompt(
    dir: &Path,
    notes: &[SharedMemoryNote],
    memory_enabled: bool,
) -> Result<(Option<String>, Option<String>), String> {
    if !memory_enabled {
        return Ok((None, Some(DISABLED_WARNING.to_string())));
    }
    let l1 = memory::read_layer_if_present(dir, MemoryLayer::L1)?;
    if let Some(layer) = &l1 {
        if layer.bytes > memory::L1_MAX_BYTES {
            return Ok((
                None,
                Some(format!(
                    "这份 Bot 的私有 L1 有 {} 字节，超过 {} 字节，本轮不注入。没有改用全局记忆。",
                    layer.bytes,
                    memory::L1_MAX_BYTES
                )),
            ));
        }
    }
    let l1_text = l1
        .as_ref()
        .map(|layer| layer.content.trim().to_string())
        .filter(|text| !text.is_empty() && text != "# L1 Online Memory");
    let note_text: Vec<&str> = notes
        .iter()
        .map(|note| note.content.trim())
        .filter(|text| !text.is_empty())
        .collect();
    if l1_text.is_none() && note_text.is_empty() {
        return Ok((None, None));
    }
    let mut body = String::from(
        "Bot private memory (this Bot only; not the global Kivio memory and not another Bot's memory):\n",
    );
    if let Some(text) = l1_text {
        body.push_str("\n");
        body.push_str(&text);
    }
    if !note_text.is_empty() {
        body.push_str("\n\nExplicitly shared notes:\n");
        for note in note_text {
            body.push_str("- ");
            body.push_str(note);
            body.push('\n');
        }
    }
    Ok((Some(body), None))
}

pub fn validate_shared_note(content: &str, existing: usize) -> Result<String, String> {
    let content = content.trim();
    if content.is_empty() {
        return Err("共享记忆需要一段明确的摘录，不会读取或复制全局记忆文件".into());
    }
    if content.chars().count() > MAX_SHARED_NOTE_CHARS {
        return Err(format!("共享摘录不能超过 {MAX_SHARED_NOTE_CHARS} 个字符"));
    }
    if existing >= MAX_SHARED_NOTES {
        return Err(format!("共享摘录最多 {MAX_SHARED_NOTES} 条"));
    }
    memory::ensure_memory_text_allowed(content)?;
    Ok(content.to_string())
}

pub fn read_private_layers(dir: &Path) -> Result<(Option<String>, Option<String>), String> {
    let l1 = memory::read_layer_if_present(dir, MemoryLayer::L1)?.map(|layer| layer.content);
    let l2 = memory::read_layer_if_present(dir, MemoryLayer::L2)?.map(|layer| layer.content);
    Ok((l1, l2))
}

pub fn write_private_layer(dir: &Path, layer: &str, content: &str) -> Result<(), String> {
    let layer = MemoryLayer::from_str(layer)?;
    memory::save_layer_at(dir, layer, content)?;
    Ok(())
}

pub fn clear_private_layer(dir: &Path, layer: &str) -> Result<(), String> {
    let layer = MemoryLayer::from_str(layer)?;
    memory::reset_layer_at(dir, layer)?;
    Ok(())
}
