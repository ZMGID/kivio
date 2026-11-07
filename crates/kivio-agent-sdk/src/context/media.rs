use super::estimate::{json_escaped_bytes, IMAGE_PART_TYPES, TEXT_PART_TYPES};
use crate::agent::history::RuntimeMessage;
use crate::model::MessagePart;
use serde_json::{json, Value};
const IMAGE_PART_PLACEHOLDER: &str = "[image attachment omitted]";

const DUPLICATE_IMAGE_PLACEHOLDER: &str = "[同一张图片已在上下文中提供，此处不重复上传]";

/// Request-copy media limit. Evict oldest unique images only when their total exceeds it.
pub const IMAGE_BYTES_BUDGET: usize = 16 * 1024 * 1024;

/// Bound media bytes and deduplicate against images retained in this request copy.
pub fn prune_image_parts(messages: &mut [Value], budget: usize) -> usize {
    use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};

    let hasher = BuildHasherDefault::<DefaultHasher>::default();

    let images = messages
        .iter_mut()
        .filter_map(|message| message.get_mut("content").and_then(Value::as_array_mut))
        .flat_map(|parts| parts.iter_mut())
        .filter(|part| {
            part.get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| IMAGE_PART_TYPES.contains(&kind))
        })
        .map(|part| {
            let serialized = part.to_string();
            let hash = hasher.hash_one(&serialized);
            (part, hash, serialized.len())
        })
        .collect();
    prune_images(images, budget, |part, text| {
        *part = json!({"type": "text", "text": text});
    })
}

pub fn prune_runtime_images(messages: &mut [RuntimeMessage], budget: usize) -> usize {
    use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};

    const IMAGE_URL_ENVELOPE: &str = r#"{"type":"image_url","image_url":{"url":""}}"#;
    let hasher = BuildHasherDefault::<DefaultHasher>::default();
    let optional_bytes = |key: &str, value: &Option<String>| {
        value.as_ref().map_or(0, |value| {
            key.len() + ",\"\":\"\"".len() + json_escaped_bytes(value)
        })
    };
    let images = messages
        .iter_mut()
        .filter_map(RuntimeMessage::model_message_mut)
        .flat_map(|message| &mut message.content)
        .filter_map(|part| {
            let (hash, bytes) = match &*part {
                MessagePart::Image {
                    mime_type,
                    data,
                    detail,
                    path,
                } if !data.is_empty() || path.is_some() => (
                    hasher.hash_one((mime_type, data, detail, path)),
                    IMAGE_URL_ENVELOPE.len()
                        + "data:;base64,".len()
                        + mime_type.len()
                        + data.len()
                        + optional_bytes("detail", detail)
                        + optional_bytes("path", path),
                ),
                MessagePart::ImageUrl { url, detail } => (
                    hasher.hash_one((url, detail)),
                    IMAGE_URL_ENVELOPE.len()
                        + json_escaped_bytes(url)
                        + optional_bytes("detail", detail),
                ),
                _ => return None,
            };
            Some((part, hash, bytes))
        })
        .collect();
    prune_images(images, budget, |part, text| {
        *part = MessagePart::Text { text };
    })
}

fn prune_images<P>(
    mut images: Vec<(P, u64, usize)>,
    budget: usize,
    replace: impl Fn(P, String),
) -> usize {
    let mut seen = std::collections::HashSet::new();
    let unique_bytes = images
        .iter()
        .filter(|(_, hash, _)| seen.insert(*hash))
        .fold(0usize, |sum, (_, _, bytes)| sum.saturating_add(*bytes));
    // Preserve the cached prefix when all distinct images fit. Under pressure,
    // select newest copies first; deduplicate only against images actually kept.
    if unique_bytes > budget {
        images.reverse();
    }
    seen.clear();
    let mut kept_bytes = 0usize;
    let mut saved = 0usize;
    for (part, hash, bytes) in images {
        if seen.contains(&hash) {
            saved += bytes;
            replace(part, DUPLICATE_IMAGE_PLACEHOLDER.into());
        } else if kept_bytes.saturating_add(bytes) > budget {
            saved += bytes;
            replace(
                part,
                format!(
                    "[图片已从上下文移除以控制请求体积（约 {}KB）。需要时请重新读取该文件。]",
                    bytes / 1024
                ),
            );
        } else {
            seen.insert(hash);
            kept_bytes += bytes;
        }
    }
    saved
}

pub(super) fn render_multimodal_content(content: &Value) -> String {
    match content {
        Value::Array(parts) => parts
            .iter()
            .map(render_content_part)
            .collect::<Vec<_>>()
            .join(" "),
        other => render_content_part(other),
    }
}

fn render_content_part(part: &Value) -> String {
    if part.get("type").and_then(Value::as_str) == Some("video_url") {
        return "[video attachment omitted]".into();
    }
    if let Some(kind) = part.get("type").and_then(Value::as_str) {
        if IMAGE_PART_TYPES.contains(&kind) {
            return IMAGE_PART_PLACEHOLDER.to_string();
        }
        if TEXT_PART_TYPES.contains(&kind) {
            return part
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
        }
    }
    part.to_string()
}
