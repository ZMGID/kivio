use base64::{engine::general_purpose, Engine as _};

use crate::mcp::{self, types::ChatToolArtifact};

/// R1（单图 ≤8MB / 单结果 ≤4 张）护栏 + 过滤的纯函数部分：从 MCP 工具结果的
/// `artifacts` 里选出可内联的图片（mime 以 `image/` 开头且 `data_url` 非空），
/// 解码 base64 校验大小与数量上限。返回已解码字节（配 `analyze_...`/直喂都要用）
/// 与超限提示文案（供追加进 tool 结果 content，让模型知道有图被跳过）。不涉及
/// IO/网络，纯同步，便于单测覆盖护栏边界；`attach_image_artifacts_for_model`
/// 是它的唯一调用方。
pub(super) fn select_image_artifacts_for_attach(
    artifacts: &[ChatToolArtifact],
    max_bytes: usize,
    max_images: usize,
) -> (Vec<(&ChatToolArtifact, Vec<u8>)>, Option<String>) {
    let mut accepted: Vec<(&ChatToolArtifact, Vec<u8>)> = Vec::new();
    let mut oversize_count = 0usize;
    let mut overflow_count = 0usize;

    for artifact in artifacts {
        if !artifact.mime_type.starts_with("image/") || artifact.data_url.is_empty() {
            continue;
        }
        let base64_payload = artifact
            .data_url
            .split_once(',')
            .map(|(_, data)| data)
            .unwrap_or(artifact.data_url.as_str());
        let Ok(bytes) = general_purpose::STANDARD.decode(base64_payload) else {
            continue; // 解不出 base64：保留原有占位符，不计入护栏统计
        };
        if bytes.len() > max_bytes {
            oversize_count += 1;
            continue;
        }
        if accepted.len() >= max_images {
            overflow_count += 1;
            continue;
        }
        accepted.push((artifact, bytes));
    }

    let mut notes = Vec::new();
    if oversize_count > 0 {
        notes.push(format!(
            "（{oversize_count} 张图片超过 {}MB 上限，未内联，保留文字占位符）",
            max_bytes / (1024 * 1024)
        ));
    }
    if overflow_count > 0 {
        notes.push(format!(
            "（另有 {overflow_count} 张图片超出单结果 {max_images} 张上限，未内联）"
        ));
    }
    let guard_note = if notes.is_empty() {
        None
    } else {
        Some(notes.join("\n"))
    };
    (accepted, guard_note)
}

pub(super) fn append_tool_result_note(result: &mut mcp::types::McpToolCallResult, note: &str) {
    if result.content.trim().is_empty() {
        result.content = note.to_string();
    } else {
        result.content = format!("{}\n\n{note}", result.content.trim_end());
    }
}

pub(super) fn image_extension_for_mime(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        "image/tiff" => "tiff",
        _ => "png",
    }
}

/// `image_content_part` 的姊妹函数：吃已经是 data: URL 的字符串（R1，MCP 工具
/// 结果的 image artifact 本身就是 data_url，不需要落盘再读一次）。内容部分的
/// JSON 形状与 `image_content_part` 保持一致，各 provider 适配器不用区分来源。
pub(super) fn data_url_image_part(data_url: &str) -> Result<serde_json::Value, String> {
    if !data_url.starts_with("data:") {
        return Err("Artifact data_url is not a data: URL".to_string());
    }
    Ok(serde_json::json!({
        "type": "image_url",
        "image_url": { "url": data_url },
    }))
}

#[cfg(test)]
mod tests {
    use base64::{engine::general_purpose, Engine as _};
    use serde_json::Value;

    use crate::mcp;

    use super::*;

    fn image_artifact(name: &str, mime_type: &str, payload: &[u8]) -> ChatToolArtifact {
        ChatToolArtifact {
            id: None,
            name: name.to_string(),
            mime_type: mime_type.to_string(),
            data_url: format!(
                "data:{mime_type};base64,{}",
                general_purpose::STANDARD.encode(payload)
            ),
            size_bytes: None,
            path: None,
        }
    }

    #[test]
    fn select_image_artifacts_filters_non_image_and_empty_data_url() {
        let artifacts = vec![
            // 非图片 artifact：直接跳过，不计入护栏统计。
            ChatToolArtifact {
                id: None,
                name: "notes.txt".to_string(),
                mime_type: "text/plain".to_string(),
                data_url: "data:text/plain;base64,aGVsbG8=".to_string(),
                size_bytes: None,
                path: None,
            },
            // data_url 为空的图片 artifact：同样跳过。
            ChatToolArtifact {
                id: None,
                name: "empty.png".to_string(),
                mime_type: "image/png".to_string(),
                data_url: String::new(),
                size_bytes: None,
                path: None,
            },
            image_artifact("shot.png", "image/png", b"tiny-png-bytes"),
        ];

        let (accepted, guard_note) =
            select_image_artifacts_for_attach(&artifacts, 8 * 1024 * 1024, 4);

        assert_eq!(accepted.len(), 1);
        assert_eq!(accepted[0].0.name, "shot.png");
        assert!(guard_note.is_none());
    }

    #[test]
    fn select_image_artifacts_no_images_passes_through_unchanged() {
        let artifacts = vec![ChatToolArtifact {
            id: None,
            name: "notes.txt".to_string(),
            mime_type: "text/plain".to_string(),
            data_url: "data:text/plain;base64,aGVsbG8=".to_string(),
            size_bytes: None,
            path: None,
        }];

        let (accepted, guard_note) =
            select_image_artifacts_for_attach(&artifacts, 8 * 1024 * 1024, 4);

        assert!(accepted.is_empty());
        assert!(guard_note.is_none());
    }

    #[test]
    fn select_image_artifacts_skips_oversize_image_with_note() {
        let artifacts = vec![image_artifact("big.png", "image/png", &[0u8; 64])];

        // max_bytes 故意设得比 payload 小，触发「过大」护栏。
        let (accepted, guard_note) = select_image_artifacts_for_attach(&artifacts, 16, 4);

        assert!(accepted.is_empty());
        let note = guard_note.expect("oversize image should produce a guard note");
        assert!(note.contains("1 张图片超过"), "note was: {note}");
    }

    #[test]
    fn select_image_artifacts_caps_at_max_images_and_notes_overflow() {
        let artifacts: Vec<ChatToolArtifact> = (0..6)
            .map(|i| {
                image_artifact(
                    &format!("img{i}.png"),
                    "image/png",
                    format!("payload-{i}").as_bytes(),
                )
            })
            .collect();

        let (accepted, guard_note) =
            select_image_artifacts_for_attach(&artifacts, 8 * 1024 * 1024, 4);

        assert_eq!(accepted.len(), 4);
        let note = guard_note.expect("overflow beyond the cap should produce a guard note");
        assert!(
            note.contains("另有 2 张图片超出单结果 4 张上限"),
            "note was: {note}"
        );
    }

    #[test]
    fn data_url_image_part_wraps_data_url_and_rejects_non_data_url() {
        let part = data_url_image_part("data:image/png;base64,AAAA").expect("valid data url");
        assert_eq!(part["type"], "image_url");
        assert_eq!(part["image_url"]["url"], "data:image/png;base64,AAAA");

        assert!(data_url_image_part("https://example.com/shot.png").is_err());
    }

    #[test]
    fn image_extension_for_mime_matches_common_types() {
        assert_eq!(image_extension_for_mime("image/jpeg"), "jpg");
        assert_eq!(image_extension_for_mime("image/webp"), "webp");
        assert_eq!(image_extension_for_mime("image/png"), "png");
        assert_eq!(image_extension_for_mime("image/unknown"), "png");
    }

    #[test]
    fn append_tool_result_note_handles_empty_and_non_empty_content() {
        let mut result = mcp::types::McpToolCallResult {
            content: String::new(),
            is_error: false,
            raw: Value::Null,
            artifacts: Vec::new(),
            structured_content: None,
            follow_up_user_messages: Vec::new(),
        };
        append_tool_result_note(&mut result, "note one");
        assert_eq!(result.content, "note one");

        append_tool_result_note(&mut result, "note two");
        assert_eq!(result.content, "note one\n\nnote two");
    }
}
