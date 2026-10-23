//! Shared memory files, byte quotas and content validation.
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub const L1_MAX_BYTES: usize = 5_000;

pub(crate) const MEMORY_DIR: &str = "chat-memory";
pub(crate) const L1_FILE: &str = "L1.md";
pub(crate) const L2_FILE: &str = "L2.md";

pub(crate) const DEFAULT_L1: &str = "# L1 Online Memory\n\n";
pub(crate) const DEFAULT_L2: &str = "# L2 Long-Term Memory\n\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryLayer {
    L1,
    L2,
}

impl MemoryLayer {
    pub fn from_str(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "l1" => Ok(Self::L1),
            "l2" => Ok(Self::L2),
            _ => Err("Memory layer must be \"l1\" or \"l2\"".to_string()),
        }
    }

    pub(crate) fn file_name(self) -> &'static str {
        match self {
            Self::L1 => L1_FILE,
            Self::L2 => L2_FILE,
        }
    }

    pub(crate) fn default_content(self) -> &'static str {
        match self {
            Self::L1 => DEFAULT_L1,
            Self::L2 => DEFAULT_L2,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::L1 => "L1",
            Self::L2 => "L2",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryLayerContent {
    pub layer: String,
    pub content: String,
    pub bytes: usize,
    pub max_bytes: Option<usize>,
}

pub fn memory_dir_at(base: &Path) -> Result<PathBuf, String> {
    let dir = base.join(MEMORY_DIR);
    fs::create_dir_all(&dir).map_err(|e| format!("create memory dir: {e}"))?;
    Ok(dir)
}

pub(crate) fn memory_file_path_at(dir: &Path, layer: MemoryLayer) -> PathBuf {
    dir.join(layer.file_name())
}

pub(crate) fn ensure_memory_file_at(dir: &Path, layer: MemoryLayer) -> Result<PathBuf, String> {
    let path = memory_file_path_at(dir, layer);
    if !path.exists() {
        crate::chat::storage::atomic_write(&path, layer.default_content(), "memory")?;
    }
    Ok(path)
}

pub fn read_layer_at(dir: &Path, layer: MemoryLayer) -> Result<MemoryLayerContent, String> {
    let path = ensure_memory_file_at(dir, layer)?;
    let content =
        fs::read_to_string(&path).map_err(|e| format!("read {} memory: {e}", layer.label()))?;
    Ok(MemoryLayerContent {
        layer: layer.label().to_ascii_lowercase(),
        bytes: content.len(),
        max_bytes: (layer == MemoryLayer::L1).then_some(L1_MAX_BYTES),
        content,
    })
}

pub fn save_layer_at(
    dir: &Path,
    layer: MemoryLayer,
    content: &str,
) -> Result<MemoryLayerContent, String> {
    validate_memory_content(layer, content)?;
    let path = memory_file_path_at(dir, layer);
    crate::chat::storage::atomic_write(&path, content, "memory")?;
    read_layer_at(dir, layer)
}

pub(crate) fn validate_memory_content(layer: MemoryLayer, content: &str) -> Result<(), String> {
    validate_secret_free(content)?;
    if layer == MemoryLayer::L1 && content.len() > L1_MAX_BYTES {
        return Err(format!(
            "L1 memory is {} bytes, over the {L1_MAX_BYTES} byte limit. Move details to L2 and keep only short active facts in L1.",
            content.len()
        ));
    }
    Ok(())
}

pub(crate) fn validate_secret_free(content: &str) -> Result<(), String> {
    let lower = content.to_ascii_lowercase();
    // 只看字段名/前缀的裸 contains 会误伤正常笔记："sk-" 命中 task-/desk-/risk-，
    // "bearer"/"password" 是普通英文词，命令/路径偏好也不应被当成凭据。
    // 只有明确的 token 前缀，或字段名后跟足够长的 token，才判为真凭据。
    // 报错点名命中的模式，避免模型盲改重试同一条内容。
    let key_like = [
        ("sk-", 16, false),
        ("ghp_", 20, false),
        ("github_pat_", 20, false),
        ("bearer", 20, true),
        ("api_key", 16, true),
        ("apikey", 16, true),
        ("secret_key", 16, true),
        ("password", 12, true),
        ("passwd", 12, true),
    ];
    for (marker, min_len, requires_separator) in key_like {
        if contains_credential(&lower, marker, min_len, requires_separator) {
            return Err(format!(
                "Memory content looks like it contains a credential (`{marker}` followed by a token); save a redacted summary instead."
            ));
        }
    }
    if lower.contains("-----begin") {
        return Err(
            "Memory content looks like it contains a private key block; save a redacted summary instead."
                .to_string(),
        );
    }
    // 「system prompt」不在列：讨论系统提示词的笔记是合法内容，注入标志是祈使句本身。
    let injection_like = [
        "ignore previous instructions",
        "ignore all previous instructions",
        "disregard previous instructions",
    ];
    for needle in injection_like {
        if lower.contains(needle) {
            return Err(format!(
                "Memory content looks like prompt-injection instructions (matched `{needle}`); save neutral facts instead."
            ));
        }
    }
    Ok(())
}

pub(crate) fn contains_credential(
    lower: &str,
    marker: &str,
    min_len: usize,
    requires_separator: bool,
) -> bool {
    let bytes = lower.as_bytes();
    let mut from = 0;
    while let Some(pos) = lower[from..].find(marker) {
        let at = from + pos;
        let boundary = at == 0
            || !(bytes[at - 1].is_ascii_alphanumeric()
                || bytes[at - 1] == b'_'
                || bytes[at - 1] == b'-');
        let mut i = at + marker.len();
        if requires_separator
            && i < bytes.len()
            && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'-')
        {
            from = at + marker.len();
            continue;
        }
        let mut skipped = 0;
        while i < bytes.len()
            && skipped < 3
            && matches!(bytes[i], b' ' | b':' | b'=' | b'"' | b'\'')
        {
            i += 1;
            skipped += 1;
        }
        let run = bytes[i..]
            .iter()
            .take_while(|b| b.is_ascii_alphanumeric() || **b == b'_' || **b == b'-')
            .count();
        if boundary && run >= min_len {
            return true;
        }
        from = at + marker.len();
    }
    false
}
